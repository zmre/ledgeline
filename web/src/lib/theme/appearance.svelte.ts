// Applies the saved appearance — theme and interface scale — to the page, and
// keeps it applied as the prefs, the OS colour scheme or the zoom shortcuts
// change it (plan 39).
//
// The prefs are the source of truth (`settings.theme`, `settings.uiScale`, kept
// in ui-prefs.toml / ledgeline.toml). They arrive by fetch, so the FIRST paint
// is decided earlier, by the inline boot script in `app.html`, from a small
// localStorage cache this module writes: the variant to use under each OS
// scheme, and the scale. The cache is never read back as a preference — only
// that boot script reads it, and the prefs overwrite whatever it guessed.

import {settings} from "#lib/stores/settings.svelte.js";
import {themeFamily, isFamilyId, type VariantId} from "./catalog.js";
import {resolveTheme} from "./resolve.js";
import {applyScale} from "./scale.js";
import {setScaleSink} from "./zoom.svelte.js";

/** localStorage key the `app.html` boot script reads. Keep the two in step. */
export const APPEARANCE_CACHE_KEY = "ledgeline.appearance";

const DARK_QUERY = "(prefers-color-scheme: dark)";

/** What the boot script needs to paint the first frame without the catalog. */
export interface AppearanceCache {
    light: VariantId;
    dark: VariantId;
    scale: number;
}

/** The variant each OS scheme would show for `pref`, plus the scale. */
export function appearanceCache(pref: string, scale: number): AppearanceCache {
    return {light: resolveTheme(pref, false), dark: resolveTheme(pref, true), scale};
}

/** Whether `pref` changes with the OS scheme (a family with both variants). */
export function followsSystem(pref: string): boolean {
    const variants = isFamilyId(pref) ? themeFamily(pref)?.variants : undefined;
    return variants?.light !== undefined && variants.dark !== undefined;
}

let prefersDark = $state(false);

/** The OS colour scheme, reactively (false where `matchMedia` is missing). */
export function systemPrefersDark(): boolean {
    return prefersDark;
}

function writeCache(cache: AppearanceCache): void {
    try {
        localStorage.setItem(APPEARANCE_CACHE_KEY, JSON.stringify(cache));
    } catch {
        // Private mode / storage full: the next launch just paints the default first.
    }
}

/**
 * Wire the appearance up. Call once, during the root layout's initialisation
 * (it registers effects). Returns nothing; teardown is the effects' own.
 */
export function useAppearance(root: HTMLElement = document.documentElement): void {
    // Zoom shortcuts and the View menu step the SAVED scale, so a zoom is a
    // preference change like any other: debounced to the prefs files, and it
    // follows the ledger pin.
    setScaleSink(
        (percent) => {
            settings.uiScale = percent;
        },
        () => settings.uiScale
    );

    $effect(() => {
        if (typeof window.matchMedia !== "function") return;
        const query = window.matchMedia(DARK_QUERY);
        prefersDark = query.matches;
        const onChange = (event: MediaQueryListEvent) => {
            prefersDark = event.matches;
        };
        query.addEventListener("change", onChange);
        return () => query.removeEventListener("change", onChange);
    });

    $effect(() => {
        root.setAttribute("data-theme", resolveTheme(settings.theme, prefersDark));
    });

    $effect(() => {
        applyScale(settings.uiScale, root);
    });

    // Only once the prefs have loaded: before that, `settings` holds defaults,
    // and caching them would undo the boot script's correct guess next launch.
    $effect(() => {
        if (!settings.uiPrefsReady) return;
        writeCache(appearanceCache(settings.theme, settings.uiScale));
    });
}
