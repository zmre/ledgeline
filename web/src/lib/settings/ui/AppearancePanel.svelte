<script lang="ts">
    // Settings → Appearance (plan 39): the theme and the interface scale.
    //
    // A family card picks the family; for a family with both a light and a dark
    // variant that means "follow the OS" (its family id), and the Auto / Light /
    // Dark buttons under it pin one variant instead. Dark-only families have one
    // id. Each card paints its own colours (the catalog's swatches, tested equal
    // to the theme files) so the grid is a preview, not a list of names.
    //
    // "Use for this ledger only" pins the key in the ledger's ledgeline.toml —
    // the one place the app may create that file, because you asked it to.
    import {settings, type ThemeId} from "#lib/stores/settings.svelte.js";
    import {systemPrefersDark} from "#lib/theme/appearance.svelte.js";
    import {THEME_FAMILIES, themeVariant, type ThemeFamily, type ThemeVariant} from "#lib/theme/catalog.js";
    import {resolveTheme} from "#lib/theme/resolve.js";
    import {DEFAULT_SCALE, SCALE_STEPS} from "#lib/theme/scale.js";
    import {zoomIn, zoomOut, zoomReset} from "#lib/theme/zoom.svelte.js";

    const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);
    const mod = isMac ? "⌘" : "Ctrl+";

    const prefersDark = $derived(systemPrefersDark());
    const selected = $derived(settings.theme);
    const shown = $derived(resolveTheme(selected, prefersDark));

    /** Whether this family is the chosen one, by family id or by one of its variants. */
    const isChosen = (f: ThemeFamily) => selected === f.id || selected === f.variants.light || selected === f.variants.dark;
    const both = (f: ThemeFamily) => f.variants.light !== undefined && f.variants.dark !== undefined;

    /** The variant a card previews: what the page shows if chosen, else what choosing it would show. */
    function preview(f: ThemeFamily): ThemeVariant | undefined {
        const id = isChosen(f) ? shown : resolveTheme(f.id, prefersDark);
        return themeVariant(id);
    }

    function choose(id: string): void {
        settings.theme = id as ThemeId;
    }

    let pinBusy = $state(false);
    async function pin(key: "theme" | "uiScale", pinned: boolean): Promise<void> {
        pinBusy = true;
        try {
            await settings.pinToLedger(key, pinned);
        } finally {
            pinBusy = false;
        }
    }

    const scale = $derived(settings.uiScale);
    const offStep = $derived(!SCALE_STEPS.includes(scale as (typeof SCALE_STEPS)[number]));
</script>

<section class="flex max-w-4xl flex-col gap-6" data-testid="settings-appearance">
    <div class="flex flex-col gap-3">
        <div class="flex flex-wrap items-baseline justify-between gap-2">
            <h2 class="text-base font-semibold">Theme</h2>
            {#if settings.canPinToLedger}
                <label class="flex cursor-pointer items-center gap-2 text-sm">
                    <input
                        type="checkbox"
                        class="checkbox checkbox-sm"
                        checked={settings.isPinned("theme")}
                        disabled={pinBusy}
                        onchange={(e) => void pin("theme", e.currentTarget.checked)}
                        data-testid="appearance-pin-theme"
                    />
                    Use for this ledger only
                </label>
            {/if}
        </div>
        <p class="text-sm text-base-content/70">
            Themes marked <span class="font-medium">Auto</span> switch between light and dark with your system setting. Pick Light or Dark to keep one.
            {#if settings.isPinned("theme")}This ledger has its own theme, kept in its <code>ledgeline.toml</code>.{/if}
        </p>

        <div class="grid grid-cols-[repeat(auto-fill,minmax(min(13rem,100%),1fr))] gap-3" role="radiogroup" aria-label="Theme">
            {#each THEME_FAMILIES as family (family.id)}
                {@const v = preview(family)}
                {@const chosen = isChosen(family)}
                <div
                    class={[
                        "flex flex-col overflow-hidden rounded-box border-2 transition-colors",
                        chosen ? "border-primary" : "border-base-300 hover:border-base-content/30",
                    ]}
                    data-testid="theme-card-{family.id}"
                >
                    <button
                        type="button"
                        role="radio"
                        aria-checked={chosen}
                        class="flex cursor-pointer flex-col gap-2 p-3 text-left"
                        style:background-color={v?.swatch.base100}
                        style:color={v?.swatch.baseContent}
                        onclick={() => choose(both(family) ? family.id : (family.variants.dark ?? family.id))}
                    >
                        <span class="flex items-center justify-between gap-2">
                            <span class="font-semibold">{family.label}</span>
                            <span class="flex gap-1" aria-hidden="true">
                                {#each [v?.swatch.primary, v?.swatch.secondary, v?.swatch.accent] as c, i (i)}
                                    <span class="size-3 rounded-full" style:background-color={c}></span>
                                {/each}
                            </span>
                        </span>
                        <span class="flex h-8 items-end gap-1" aria-hidden="true">
                            {#each v?.swatch.chart ?? [] as c, i (i)}
                                <span class="w-3 rounded-t-sm" style:background-color={c} style:height="{[55, 90, 40, 70][i] ?? 50}%"></span>
                            {/each}
                            <span class="ml-auto text-xs opacity-70">Aa 1,234.56</span>
                        </span>
                    </button>
                    {#if both(family)}
                        <div class="join w-full rounded-none border-t border-base-300 bg-base-100" role="group" aria-label="{family.label} variant">
                            {#each [{id: family.id, label: "Auto"}, {id: family.variants.light!, label: "Light"}, {id: family.variants.dark!, label: "Dark"}] as opt (opt.id)}
                                <button
                                    type="button"
                                    class={["btn join-item flex-1 rounded-none btn-xs", selected === opt.id ? "btn-primary" : "btn-ghost"]}
                                    aria-pressed={selected === opt.id}
                                    title={opt.label === "Auto" ? "Follow the system light/dark setting" : (themeVariant(opt.id)?.label ?? opt.label)}
                                    onclick={() => choose(opt.id)}
                                    data-testid="theme-option-{opt.id}">{opt.label}</button
                                >
                            {/each}
                        </div>
                    {/if}
                </div>
            {/each}
        </div>
    </div>

    <div class="flex flex-col gap-3">
        <div class="flex flex-wrap items-baseline justify-between gap-2">
            <h2 class="text-base font-semibold">Text size</h2>
            {#if settings.canPinToLedger}
                <label class="flex cursor-pointer items-center gap-2 text-sm">
                    <input
                        type="checkbox"
                        class="checkbox checkbox-sm"
                        checked={settings.isPinned("uiScale")}
                        disabled={pinBusy}
                        onchange={(e) => void pin("uiScale", e.currentTarget.checked)}
                        data-testid="appearance-pin-scale"
                    />
                    Use for this ledger only
                </label>
            {/if}
        </div>
        <p class="text-sm text-base-content/70">
            Scales all text and the layout around it.<span class="key-hint">
                Shortcuts: <kbd class="kbd kbd-sm">{mod}+</kbd> larger, <kbd class="kbd kbd-sm">{mod}−</kbd> smaller, <kbd class="kbd kbd-sm">{mod}0</kbd> actual
                size.</span
            >
        </p>
        <div class="flex flex-wrap items-center gap-2">
            <button type="button" class="btn btn-square btn-sm" aria-label="Smaller" onclick={() => zoomOut()} disabled={scale <= SCALE_STEPS[0]}>−</button>
            <select
                class="select w-28 select-sm"
                aria-label="Text size"
                value={String(scale)}
                onchange={(e) => (settings.uiScale = Number(e.currentTarget.value))}
                data-testid="appearance-scale"
            >
                {#if offStep}<option value={String(scale)}>{scale}%</option>{/if}
                {#each SCALE_STEPS as step (step)}
                    <option value={String(step)}>{step}%</option>
                {/each}
            </select>
            <button
                type="button"
                class="btn btn-square btn-sm"
                aria-label="Larger"
                onclick={() => zoomIn()}
                disabled={scale >= SCALE_STEPS[SCALE_STEPS.length - 1]}>+</button
            >
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => zoomReset()} disabled={scale === DEFAULT_SCALE}>Reset</button>
        </div>
    </div>
</section>
