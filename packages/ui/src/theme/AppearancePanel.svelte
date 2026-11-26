<!-- Settings → Appearance (plan 39), shared by Ledgeline and the invoicer: the
     theme and the interface scale. Both apply at once; there is no Save.

     A family card picks the family; for a family with both a light and a dark
     variant that means "follow the OS" (its family id), and the Auto / Light /
     Dark buttons under it pin one variant instead. Dark-only families have one
     id. Each card paints its own colours (the catalog's swatches, tested equal
     to the theme files) so the grid is a preview, not a list of names.

     The app owns the values and where they are saved: it hands in the current
     theme and scale, what to do when one is chosen, and the per-books pin
     ("Use for this ledger only"), which is offered only where the app can
     keep a setting for the open books (`canPin`). The zoom buttons step the
     scale through the app's scale sink (`useAppearance`), as the keys do. -->
<script lang="ts" module>
    /** The appearance keys that can be kept for one set of books. */
    export type AppearanceKey = "theme" | "uiScale";
</script>

<script lang="ts">
    import type {Snippet} from "svelte";
    import PinToggle from "../components/PinToggle.svelte";
    import {systemPrefersDark} from "./appearance.svelte.js";
    import {THEME_FAMILIES, themeVariant, type ThemeFamily, type ThemeVariant} from "./catalog.js";
    import {resolveTheme} from "./resolve.js";
    import {DEFAULT_SCALE, SCALE_STEPS} from "./scale.js";
    import {zoomIn, zoomOut, zoomReset} from "./zoom.svelte.js";

    let {
        theme,
        uiScale,
        onTheme,
        onScale,
        canPin = false,
        isPinned = () => false,
        pin = async () => undefined,
        pinLabel = "Use for this ledger only",
        themePinnedNote,
        intro,
    }: {
        /** The theme preference: a family id (follows the OS) or one variant. */
        theme: string;
        /** The interface scale, in percent. */
        uiScale: number;
        onTheme: (id: string) => void;
        onScale: (percent: number) => void;
        /** Whether a setting can be kept for the open books alone. */
        canPin?: boolean;
        isPinned?: (key: AppearanceKey) => boolean;
        /** Keep `key` for the open books only, or stop. */
        pin?: (key: AppearanceKey, pinned: boolean) => Promise<unknown>;
        pinLabel?: string;
        /** Said under the theme heading while the theme is pinned. */
        themePinnedNote?: Snippet;
        /** Above everything: what this panel shares, and with what. */
        intro?: Snippet;
    } = $props();

    const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);
    const mod = isMac ? "⌘" : "Ctrl+";

    const prefersDark = $derived(systemPrefersDark());
    const shown = $derived(resolveTheme(theme, prefersDark));

    /** Whether this family is the chosen one, by family id or by one of its variants. */
    const isChosen = (f: ThemeFamily) => theme === f.id || theme === f.variants.light || theme === f.variants.dark;
    const both = (f: ThemeFamily) => f.variants.light !== undefined && f.variants.dark !== undefined;

    /** The variant a card previews: what the page shows if chosen, else what choosing it would show. */
    function preview(f: ThemeFamily): ThemeVariant | undefined {
        const id = isChosen(f) ? shown : resolveTheme(f.id, prefersDark);
        return themeVariant(id);
    }

    let pinBusy = $state(false);
    async function setPin(key: AppearanceKey, pinned: boolean): Promise<void> {
        pinBusy = true;
        try {
            await pin(key, pinned);
        } finally {
            pinBusy = false;
        }
    }

    const offStep = $derived(!SCALE_STEPS.includes(uiScale as (typeof SCALE_STEPS)[number]));
</script>

<section class="flex max-w-4xl flex-col gap-6" data-testid="settings-appearance">
    {@render intro?.()}
    <div class="flex flex-col gap-3">
        <div class="flex flex-wrap items-baseline justify-between gap-2">
            <h2 class="text-base font-semibold">Theme</h2>
            {#if canPin}
                <PinToggle
                    pinned={isPinned("theme")}
                    busy={pinBusy}
                    label={pinLabel}
                    onChange={(on) => void setPin("theme", on)}
                    testid="appearance-pin-theme"
                />
            {/if}
        </div>
        <p class="text-sm text-base-content/70">
            Themes marked <span class="font-medium">Auto</span> switch between light and dark with your system setting. Pick Light or Dark to keep one.
            {#if isPinned("theme")}{@render themePinnedNote?.()}{/if}
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
                        onclick={() => onTheme(both(family) ? family.id : (family.variants.dark ?? family.id))}
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
                                    class={["btn join-item flex-1 rounded-none btn-xs", theme === opt.id ? "btn-primary" : "btn-ghost"]}
                                    aria-pressed={theme === opt.id}
                                    title={opt.label === "Auto" ? "Follow the system light/dark setting" : (themeVariant(opt.id)?.label ?? opt.label)}
                                    onclick={() => onTheme(opt.id)}
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
            {#if canPin}
                <PinToggle
                    pinned={isPinned("uiScale")}
                    busy={pinBusy}
                    label={pinLabel}
                    onChange={(on) => void setPin("uiScale", on)}
                    testid="appearance-pin-scale"
                />
            {/if}
        </div>
        <p class="text-sm text-base-content/70">
            Scales all text and the layout around it.<span class="key-hint">
                Shortcuts: <kbd class="kbd kbd-sm">{mod}+</kbd> larger, <kbd class="kbd kbd-sm">{mod}−</kbd> smaller, <kbd class="kbd kbd-sm">{mod}0</kbd> actual
                size.</span
            >
        </p>
        <div class="flex flex-wrap items-center gap-2">
            <button type="button" class="btn btn-square btn-sm" aria-label="Smaller" onclick={() => zoomOut()} disabled={uiScale <= SCALE_STEPS[0]}>−</button>
            <select
                class="select w-28 select-sm"
                aria-label="Text size"
                value={String(uiScale)}
                onchange={(e) => onScale(Number(e.currentTarget.value))}
                data-testid="appearance-scale"
            >
                {#if offStep}<option value={String(uiScale)}>{uiScale}%</option>{/if}
                {#each SCALE_STEPS as step (step)}
                    <option value={String(step)}>{step}%</option>
                {/each}
            </select>
            <button
                type="button"
                class="btn btn-square btn-sm"
                aria-label="Larger"
                onclick={() => zoomIn()}
                disabled={uiScale >= SCALE_STEPS[SCALE_STEPS.length - 1]}>+</button
            >
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => zoomReset()} disabled={uiScale === DEFAULT_SCALE}>Reset</button>
        </div>
    </div>
</section>
